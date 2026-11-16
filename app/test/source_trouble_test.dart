// A source that cannot start says why in the composer, with the one thing to do: Install when its program is missing, Sign in
// when nobody is signed in, Try again when it could not be asked. Sending to a source of the first two opens that action instead
// of starting. The host's answers are scripted: the kinds are those `canoa options` prints (host_options_test is the real one).
import 'dart:convert';
import 'dart:io' show Directory;

import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/add_account_dialog.dart';
import 'package:canoa/chat/composer.dart';
import 'package:canoa/chat/source_trouble.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/source_choices.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_accounts.dart';
import 'fixtures/source_choices.dart';
import 'fixtures/store_link.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _missing = 'Claude Code could not be asked: Claude Code could not be started: No such file or directory (os error 2)';
const _exited = 'Claude Code could not be asked: Claude Code exited with code 1 before completing the session';

SourceChoices _choices(Map<String, Object?> answer) => SourceChoices.parse(jsonEncode(answer))!;

final _notInstalled = _choices({'known': false, 'unknown_because': _missing, 'unknown_kind': 'not_installed', 'models': <Object?>[]});
final _failed = _choices({'known': false, 'unknown_because': _exited, 'unknown_kind': 'failed', 'models': <Object?>[]});
final _signedOut = _choices({...claudeAnswer, 'signed_out': true});

/// A loader that answers [answers] in turn for Claude (the last one for ever), and the fixtures for any other source.
final class _Host {
  _Host(this.answers);

  final List<SourceChoices> answers;
  int asked = 0;

  Future<SourceChoices?> call(AgentSource source) async {
    if (source.label != 'Claude') return fixtureChoices(source.label);
    final answer = answers[asked < answers.length ? asked : answers.length - 1];
    asked++;
    return answer;
  }
}

SessionStore _store() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

Future<void> _claude(WidgetTester tester) async {
  await newSession(tester);
  await chooseSource(tester, 'Claude');
  await openFolder(tester);
  await settle(tester);
}

String _text(WidgetTester tester, String key) => [
  for (final element in find.descendant(of: find.byKey(Key(key)), matching: find.byType(Text)).evaluate()) (element.widget as Text).data,
].join();

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/source_trouble_test.dart')));

  test('the kinds and the sign-in state are read from what canoa options prints', () {
    expect(_notInstalled.unknownKind, UnknownKind.notInstalled);
    expect(_notInstalled.known, isFalse);
    expect(_failed.unknownKind, UnknownKind.failed);
    expect(_signedOut.signedOut, isTrue);
    expect(_signedOut.known, isTrue);
    expect(_signedOut.unknownKind, isNull);
    expect(_choices(claudeAnswer).signedOut, isFalse);
    expect(_choices({'known': false, 'unknown_because': 'lists in its session', 'models': <Object?>[]}).unknownKind, isNull);
    expect(_choices({'known': false, 'unknown_kind': 'something new', 'models': <Object?>[]}).unknownKind, isNull, reason: 'a newer host\'s kind is no kind');
  });

  testWidgets('a missing program says so, replaces "after start", and offers no Install where the host grants none', (tester) async {
    await pumpApp(tester, _store(), askChoices: _Host([_notInstalled]).call);
    await _claude(tester);
    expect(_text(tester, 'source-trouble'), "Claude Code isn't installed on this host");
    expect(find.text('Model: after start'), findsNothing);
    expect(find.byKey(const Key('source-trouble-action')), findsNothing, reason: 'no Store grant: the text only');
    final tip = tester.widget<Tooltip>(find.ancestor(of: find.byKey(const Key('source-trouble')), matching: find.byType(Tooltip)).first);
    expect(tip.message, _missing, reason: 'the host\'s words are in the tooltip');
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/source_trouble_not_installed.png'));
    await finish(tester);
  });

  testWidgets('sending to a source whose program is missing starts nothing and says why', (tester) async {
    await pumpApp(tester, _store(), askChoices: _Host([_notInstalled]).call);
    await _claude(tester);
    await send(tester, 'Say hello');
    await settle(tester);
    expect(find.textContaining("Claude Code isn't installed on this host."), findsWidgets);
    expect(find.text('Say hello'), findsOneWidget, reason: 'the message stays in the composer');
    await finish(tester);
  });

  testWidgets('a signed-out source keeps its models and offers Sign in, which runs the source\'s login flow as Accounts does', (tester) async {
    final host = FakeAccountsHost(accounts: []);
    await pumpApp(tester, _store(), askChoices: _Host([_signedOut]).call, accountsHost: host);
    await _claude(tester);
    expect(_text(tester, 'source-trouble'), 'Claude needs a sign-in');
    expect(find.byKey(const Key('source-trouble-action')), findsOneWidget);
    expect(find.text('Model: after start'), findsNothing);
    expect(find.byKey(const Key('option-model')), findsOneWidget, reason: 'its models were listed');
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/source_trouble_sign_in.png'));
    await tester.tap(find.byKey(const Key('source-trouble-action')));
    await frames(tester);
    expect(find.byType(AddAccountDialog), findsOneWidget);
    expect(tester.widget<AddAccountDialog>(find.byType(AddAccountDialog)).harness, 'claude');
    await finish(tester);
  });

  testWidgets('sending to a signed-out source opens Sign in and starts nothing', (tester) async {
    final host = FakeAccountsHost(accounts: []);
    final asking = _Host([_signedOut]);
    await pumpApp(tester, _store(), askChoices: asking.call, accountsHost: host);
    await _claude(tester);
    final before = asking.asked;
    await send(tester, 'Say hello');
    await settle(tester);
    expect(asking.asked, greaterThan(before), reason: 'the host is asked once more before the send is refused');
    expect(find.byType(AddAccountDialog), findsOneWidget);
    await finish(tester);
  });

  testWidgets('a source that was signed out and is not now sends: it was asked again first', (tester) async {
    final host = FakeAccountsHost(accounts: []);
    final asking = _Host([_signedOut, _choices(claudeAnswer)]);
    await pumpApp(tester, _store(), askChoices: asking.call, accountsHost: host);
    await _claude(tester);
    expect(find.byKey(const Key('source-trouble')), findsOneWidget);
    await send(tester, 'Say hello');
    await settle(tester);
    expect(find.byType(AddAccountDialog), findsNothing);
    expect(find.byKey(const Key('source-trouble')), findsNothing, reason: 'the second answer has a login');
    await finish(tester);
  });

  testWidgets('an account the entry offers carries its own login, so the machine\'s sign-in is not asked for', (tester) async {
    final host = FakeAccountsHost(accounts: [account('work', kind: 'claude-login')]);
    const config = CanoaConfig(
      agents: [scripted],
      host: {
        'agents': {
          'Claude': {'harness': 'claude', 'accounts': ['work']},
        },
      },
    );
    await pumpApp(tester, _store(), askChoices: _Host([_signedOut]).call, accountsHost: host, config: config);
    await _claude(tester);
    expect(find.byKey(const Key('source-trouble')), findsNothing);
    expect(find.byKey(const Key('option-model')), findsOneWidget);
    await finish(tester);
  });

  testWidgets('a source that could not be asked offers Try again, which asks the host again', (tester) async {
    final asking = _Host([_failed, _choices(claudeAnswer)]);
    await pumpApp(tester, _store(), askChoices: asking.call);
    await _claude(tester);
    expect(_text(tester, 'source-trouble'), "Couldn't start Claude");
    expect(find.text('Try again'), findsOneWidget);
    expect(find.text('Model: after start'), findsNothing);
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/source_trouble_failed.png'));
    await tester.tap(find.byKey(const Key('source-trouble-action')));
    await settle(tester);
    expect(asking.asked, 2);
    expect(find.byKey(const Key('source-trouble')), findsNothing);
    expect(find.byKey(const Key('option-model')), findsOneWidget);
    await finish(tester);
  });

  testWidgets('a source that could not be asked is still sent to: starting says what failed itself', (tester) async {
    final asking = _Host([_failed]);
    await pumpApp(tester, _store(), askChoices: asking.call);
    await _claude(tester);
    expect(sourceTrouble(const AgentSource.hosted('Claude', harness: 'claude', local: true), _failed)?.blocksStart, isFalse);
    await finish(tester);
  });

  testWidgets('Install opens the Store\'s programs where the host grants installing, and asks the host again after', (tester) async {
    final fake = FakeStoreHost([claudeLike()]);
    final programs = fake.programs();
    final asking = _Host([_choices(claudeAnswer)]);
    final store = _store()..choicesLoader = asking.call;
    const source = AgentSource.hosted('Claude', harness: 'claude', local: true);
    final kit = ComposerKit(programs: programs);
    final trouble = sourceTrouble(source, _notInstalled)!;
    expect(trouble.kind, TroubleKind.notInstalled);
    expect(canAct(trouble, kit), isTrue);
    expect(canAct(trouble, const ComposerKit()), isFalse);
    await tester.pumpWidget(MaterialApp(
      home: Builder(builder: (context) => TextButton(
        key: const Key('go'),
        onPressed: () => actOnTrouble(context, store, kit, trouble),
        child: const Text('go'),
      )),
    ));
    await tester.tap(find.byKey(const Key('go')));
    await tester.pumpAndSettle();
    expect(find.text('Agent programs'), findsOneWidget);
    expect(find.text('Install'), findsWidgets, reason: 'the program\'s tile has its Install');
    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(asking.asked, 1, reason: 'asked again once the page closed');
    expect(store.choicesOf(source)?.unknownKind, isNull);
    store.dispose();
  });
}
