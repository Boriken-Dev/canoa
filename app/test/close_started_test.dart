// Closing a conversation whose agent is running ends that agent, which takes real input and output: a widget test runs the
// close outside its own clock, or the close never finishes.
import 'dart:io';

import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

void main() {
  testWidgets('a started conversation is closed outside the test clock, and its agent ends', (tester) async {
    final documents = Directory.systemTemp.createTempSync('canoa-close');
    addTearDown(() => documents.deleteSync(recursive: true));
    final store = SessionStore();
    final agent = OptionsAgent();
    await pumpApp(tester, store, documents: documents.path, linked: agent.openLink);
    await newSession(tester);
    final draft = store.selected!;
    await chooseSource(tester, 'Scripted');
    await send(tester, 'hello');
    await until(tester, () => draft.remoteSessionId != null, 'the session to start');

    await closeSession(tester, store, draft);
    expect(store.sessions, isNot(contains(draft)));
    await finish(tester);
  }, timeout: const Timeout(Duration(seconds: 60)));

  testWidgets('the same close inside the test clock does not finish', (tester) async {
    final documents = Directory.systemTemp.createTempSync('canoa-close');
    addTearDown(() => documents.deleteSync(recursive: true));
    final store = SessionStore();
    final agent = OptionsAgent();
    await pumpApp(tester, store, documents: documents.path, linked: agent.openLink);
    await newSession(tester);
    final draft = store.selected!;
    await chooseSource(tester, 'Scripted');
    await send(tester, 'hello');
    await until(tester, () => draft.remoteSessionId != null, 'the session to start');

    var done = false;
    store.close(draft).then((_) => done = true);
    await tester.pump(const Duration(seconds: 5));
    await tester.pump(const Duration(seconds: 5));
    expect(done, isFalse, reason: 'its real teardown needs the real clock');
    await settle(tester, 200);
    expect(done, isTrue, reason: 'and finishes once real time passes');
    await finish(tester);
  }, timeout: const Timeout(Duration(seconds: 60)));
}
