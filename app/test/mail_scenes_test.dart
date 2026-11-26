// Mail as Canoa draws it: Settings › Mail with mailboxes in each state and a test's answer, and the add dialog. Goldens are drawn in the
// test font; with CANOA_SAMPLES_DIR set the same scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_mail.dart';
import 'fixtures/settings_fixture.dart';
import 'fixtures/store_fixtures.dart';

FakeMailService _service() => FakeMailService(
  accounts: [
    mailAccount('work', user: 'EXAMPLE\\ana'),
    mailAccount('late', user: 'ana', state: 'needs a password'),
    mailAccount('desktop', backend: 'outlook', server: null, state: 'signed in by Outlook'),
  ],
  tests: {'late': {'ok': false, 'problem': 'The account has no stored password: add it again.'}},
);

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/mail_scenes_test.dart')));

  for (final (label, size) in [('desktop', const Size(1280, 800)), ('phone', const Size(390, 844))]) {
    group('at $label width', () {
      Future<void> open(WidgetTester tester) async {
        await openSettings(tester, size: size, section: 'mail', version: '0.0.1', hostSections: _service().sections());
      }

      Future<void> golden(WidgetTester tester, String name) async {
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/mail_${name}_$label.png'));
      }

      testWidgets('Settings, Mail: each mailbox with where it is and whether its password is there, and what a test found', (tester) async {
        await open(tester);
        await tester.tap(find.byKey(const Key('section-mail-test work')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('section-mail-test late')));
        await tester.pumpAndSettle();
        await golden(tester, 'settings');
      });

      testWidgets('Add a mailbox: the server, the user and the password', (tester) async {
        await open(tester);
        await tester.tap(find.byKey(const Key('section-mail-add')));
        await tester.pumpAndSettle();
        await tester.enterText(find.byKey(const Key('section-mail-name')), 'home');
        await tester.enterText(find.byKey(const Key('section-mail-field-server')), 'https://mail.example.test/EWS/Exchange.asmx');
        await tester.enterText(find.byKey(const Key('section-mail-field-user')), 'EXAMPLE\\ana');
        await tester.enterText(find.byKey(const Key('section-mail-field-password')), 'not shown');
        FocusManager.instance.primaryFocus?.unfocus();
        await tester.pumpAndSettle();
        await golden(tester, 'add');
      });
    });
  }
}
