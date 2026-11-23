// Mail as Canoa draws it: Settings › Mail with mailboxes in each state and a test's answer, and the add dialog. Goldens are drawn in the
// test font; with CANOA_SAMPLES_DIR set the same scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:canoa/mail/mail_accounts.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_mail.dart';
import 'fixtures/sections_fixture.dart';
import 'fixtures/settings_fixture.dart';
import 'fixtures/store_fixtures.dart';

FakeMailHost _host() => FakeMailHost(
  accounts: [
    mailAccount('work', user: 'EXAMPLE\\ana'),
    mailAccount('late', user: 'ana', state: 'needs a password'),
    mailAccount('desktop', backend: 'outlook', server: null, state: 'signed in by Outlook'),
  ],
  tests: {'late': const MailTest(ok: false, problem: 'The account has no stored password: add it again.')},
);

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/mail_scenes_test.dart')));

  for (final (label, size) in [('desktop', const Size(1280, 800)), ('phone', const Size(390, 844))]) {
    group('at $label width', () {
      Future<MailAccountsController> open(WidgetTester tester) async {
        final controller = MailAccountsController(host: _host());
        await controller.refresh();
        await openSettings(tester, size: size, section: 'mail', mail: controller, version: '0.0.1', hostSections: sectionsWithMail());
        return controller;
      }

      Future<void> golden(WidgetTester tester, String name) async {
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/mail_${name}_$label.png'));
      }

      testWidgets('Settings, Mail: each mailbox with where it is and whether its password is there, and what a test found', (tester) async {
        await open(tester);
        await tester.tap(find.byKey(const Key('mail-test work')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('mail-test late')));
        await tester.pumpAndSettle();
        await golden(tester, 'settings');
      });

      testWidgets('Add a mailbox: the server, the user and the password', (tester) async {
        await open(tester);
        await tester.tap(find.byKey(const Key('add-mail-account')));
        await tester.pumpAndSettle();
        await tester.enterText(find.byKey(const Key('mail-name')), 'home');
        await tester.enterText(find.byKey(const Key('mail-field-server')), 'https://mail.example.test/EWS/Exchange.asmx');
        await tester.enterText(find.byKey(const Key('mail-field-user')), 'EXAMPLE\\ana');
        await tester.enterText(find.byKey(const Key('mail-field-password')), 'not shown');
        FocusManager.instance.primaryFocus?.unfocus();
        await tester.pumpAndSettle();
        await golden(tester, 'add');
      });
    });
  }
}
