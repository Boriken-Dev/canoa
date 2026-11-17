// What a session relates to, how it is keyed for a webpage, and how a page asks for the chat alone.
import 'package:canoa/client/client_mode.dart';
import 'package:canoa/client/related_rail.dart';
import 'package:canoa/client/relation.dart';
import 'package:canoa/client/relation_source.dart';
import 'package:canoa/ledger/running_sessions.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  railTests();

  group('originRelation', () {
    test('keys a page by its origin and scopes it by its path', () {
      final relation = originRelation('https://Example.COM/a/b?q=1#top', title: '  Docs  ')!;
      expect(relation.kind, Relation.originKind);
      expect(relation.key, 'https://example.com');
      expect(relation.scope, 'https://example.com/a/b');
      expect(relation.label, 'Docs');
      expect(relation.display, 'example.com');
    });

    test('a default port is the same origin, another port or scheme is not', () {
      final plain = originRelation('https://example.com/')!;
      expect(originRelation('https://example.com:443/x')!.key, plain.key);
      expect(originRelation('https://example.com:8443/')!.key, 'https://example.com:8443');
      expect(originRelation('https://example.com:8443/')!.display, 'example.com:8443');
      expect(originRelation('http://example.com/')!.key, 'http://example.com');
      expect(originRelation('http://example.com/')!.display, 'http://example.com');
      expect(originRelation('http://example.com:80/')!.key, 'http://example.com');
      expect(plain.relates(originRelation('https://example.com:8443/')), isFalse);
      expect(plain.relates(originRelation('http://example.com/')), isFalse);
      expect(plain.relates(originRelation('https://www.example.com/')), isFalse);
    });

    test('a trailing slash does not make another page', () {
      expect(originRelation('https://example.com/a/')!.scope, originRelation('https://example.com/a')!.scope);
      expect(originRelation('https://example.com')!.scope, 'https://example.com/');
    });

    test('a page without a host name that is safe as a folder has no relation', () {
      for (final address in ['chrome://extensions/', 'file:///C:/a.html', 'about:blank', 'https://[::1]/', 'https://con/', 'javascript:alert(1)', '']) {
        expect(originRelation(address), isNull, reason: address);
      }
    });
  });

  group('Relation', () {
    const mail = Relation(kind: Relation.conversationKind, key: 'abc', label: 'Quarterly numbers');

    test('relates by kind and key, whatever the label', () {
      expect(mail.relates(const Relation(kind: Relation.conversationKind, key: 'abc')), isTrue);
      expect(mail.relates(const Relation(kind: Relation.conversationKind, key: 'abd')), isFalse);
      expect(mail.relates(const Relation(kind: Relation.documentKind, key: 'abc')), isFalse, reason: 'another kind');
      expect(mail.relates(null), isFalse);
    });

    test('with the narrow choice and a scope, only the same scope relates', () {
      final here = originRelation('https://example.com/a')!;
      expect(here.relates(originRelation('https://example.com/b')), isTrue);
      expect(here.relates(originRelation('https://example.com/b'), narrow: true), isFalse);
      expect(here.relates(originRelation('https://example.com/a/'), narrow: true), isTrue);
      // Nothing is narrower for a relation with no scope, so the choice changes nothing.
      expect(mail.relates(const Relation(kind: Relation.conversationKind, key: 'abc'), narrow: true), isTrue);
    });

    test('is read back as it was written, and a broken record is none', () {
      final page = originRelation('https://example.com/a', title: 'Docs')!;
      expect(Relation.fromJson(page.toJson()), page);
      expect(Relation.fromJson(mail.toJson()), mail);
      for (final broken in <Object?>[null, 'x', 3, <String, Object?>{}, {'kind': 'origin'}, {'key': 'k'}, {'kind': '', 'key': 'k'}]) {
        expect(Relation.fromJson(broken), isNull, reason: '$broken');
      }
    });

    test('shows the label for what is not a page, else the key', () {
      expect(mail.display, 'Quarterly numbers');
      expect(const Relation(kind: Relation.documentKind, key: 'h-1').display, 'h-1');
    });
  });

  group('ClientMode', () {
    test('asks for the whole app unless the address says chat', () {
      expect(ClientMode.fromUri(Uri.parse('chrome-extension://abc/ui/index.html?view=panel')).chatOnly, isFalse);
      expect(ClientMode.fromUri(Uri.parse('chrome-extension://abc/ui/index.html?view=full')).chatOnly, isFalse);
      expect(ClientMode.fromUri(Uri.parse('https://canoa.example/?mode=other')).chatOnly, isFalse);
      expect(ClientMode.fromUri(Uri.parse('chrome-extension://abc/ui/index.html?view=panel&mode=chat')).chatOnly, isTrue);
    });

    test('reads what is in front from the address, when the client knows it at load', () {
      final mode = ClientMode.fromUri(Uri.parse('https://canoa.example/?mode=chat&relation=conversation:AAMk%3D&label=Re:%20Budget&scope=s'));
      expect(mode.relation, const Relation(kind: Relation.conversationKind, key: 'AAMk=', scope: 's', label: 'Re: Budget'));
      expect(ClientMode.fromUri(Uri.parse('https://canoa.example/?mode=chat')).relation, isNull);
      expect(ClientMode.fromUri(Uri.parse('https://canoa.example/?mode=chat&relation=nokind')).relation, isNull);
      expect(ClientMode.fromUri(Uri.parse('https://canoa.example/?relation=document:x')).chatOnly, isFalse);
    });

    test('a link to the whole app drops the mode and the relation and keeps the rest', () {
      final link = ClientMode.fullAppOf(Uri.parse('chrome-extension://abc/ui/index.html?view=panel&mode=chat&relation=origin:https://a.test&label=A&look=canoa#x'));
      expect(link.path, '/ui/index.html');
      expect(link.queryParameters, {'view': 'full', 'look': 'canoa'});
      expect(link.hasFragment, isFalse);
      expect(ClientMode.fullAppOf(Uri.parse('https://canoa.example/app/')).queryParameters, {'view': 'full'});
    });
  });
}

void railTests() {
  testWidgets('the chat-only index offers the sessions the host keeps running to join, below the page\'s own', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    ledger.record(
      peerId: 'local:Claude', sessionId: 'old', entry: 'Claude', place: SessionPlace.local, reopenBy: 'session/load', cwd: '/w', title: 'An old one',
    );
    await ledger.flush();
    final store = SessionStore(ledger: ledger);
    addTearDown(store.dispose);
    final running = RunningSessions(() async => [
      RunningSession.parse({'entry': 'Claude', 'session': 's1', 'title': 'Fix the build', 'cwd': '/w', 'cursor': '9'})!,
    ]);
    addTearDown(running.dispose);
    final source = ManualRelation(originRelation('https://example.com/a'));
    addTearDown(source.dispose);
    final joined = <String>[];
    await tester.pumpWidget(MaterialApp(
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(body: RelatedRail(
        source: source, store: store, ledger: ledger, running: running,
        onAttach: (session) async => joined.add(session.sessionId),
        onNew: (_) async {}, onNewSession: () {}, onResume: (_) async {},
      )),
    ));
    await tester.pump();
    await tester.pump();

    expect(find.text('Running now'), findsOneWidget);
    expect(find.text('Fix the build'), findsOneWidget);
    expect(find.text('An old one'), findsOneWidget);
    expect(tester.getTopLeft(find.byKey(const Key('related-name'))).dy, lessThan(tester.getTopLeft(find.text('Running now')).dy));
    await tester.tap(find.byTooltip('Join'));
    await tester.pump();
    expect(joined, ['s1']);
    await tester.pump(SessionLedger.saveDelay);
  });
}
